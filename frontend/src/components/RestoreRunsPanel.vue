<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { RotateCcw } from '@lucide/vue'
import { cancelRestoreRun, listRestoreRuns } from '../api/restores'
import { useToast } from '../composables/useToast'
import { useWebSocket } from '../composables/useWebSocket'
import type { RestoreRun } from '../types/generated'
import { badgeClass } from '../utils/badge'
import { extractError } from '../utils/error'
import { formatDate } from '../utils/format'
import { logger } from '../utils/logger'
import { restoreScope, restoreStatusLabel, restoreStatusTone } from '../utils/restoreRun'
import BaseSpinner from './BaseSpinner.vue'
import EmptyState from './EmptyState.vue'

/**
 * Restores of archive files onto agents, newest first, kept current from
 * the server's `RestoreRunChanged` pushes.
 */

const runs = ref<RestoreRun[]>([])
const loading = ref(true)
const cancelling = ref<string | null>(null)
const { error: toastError } = useToast()
const { onMessage, status } = useWebSocket()

async function load(): Promise<void> {
  try {
    runs.value = await listRestoreRuns()
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    loading.value = false
  }
}

/** Puts `run` in place of its older self, or at the top if it is new. */
function upsert(run: RestoreRun): void {
  const index = runs.value.findIndex((r) => r.id === run.id)
  runs.value =
    index === -1 ? [run, ...runs.value] : runs.value.map((r, i) => (i === index ? run : r))
}

async function cancel(run: RestoreRun): Promise<void> {
  cancelling.value = run.id
  try {
    upsert(await cancelRestoreRun(run.id))
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    cancelling.value = null
  }
}

onMessage('RestoreRunChanged', (payload) => upsert(payload.run))

// Pushes sent while the UI WebSocket was down are lost.
watch(
  () => status.value,
  (now) => {
    if (now === 'connected') load().catch(logger.error)
  },
)

onMounted(() => {
  load().catch(logger.error)
})
</script>

<template>
  <BaseSpinner
    v-if="loading"
    size="lg"
  />

  <EmptyState
    v-else-if="runs.length === 0"
    :icon="RotateCcw"
    title="No restores"
    description="Restores of archive files onto an agent will appear here."
  />

  <div
    v-else
    class="table-wrap table-wrap--framed"
  >
    <table
      class="data-table"
      data-testid="restore-runs"
    >
      <thead>
        <tr>
          <th>Requested</th>
          <th>Host</th>
          <th>Archive</th>
          <th>Restoring</th>
          <th>Status</th>
          <th>By</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="run in runs"
          :key="run.id"
        >
          <td class="cell-date">
            {{ formatDate(run.created_at) }}
          </td>
          <td class="cell-host">
            {{ run.hostname }}
          </td>
          <td class="cell-mono">{{ run.repo_name }}::{{ run.archive_name }}</td>
          <td class="cell-truncate">
            <span class="cell-mono">{{ restoreScope(run) }}</span>
            into
            <span class="cell-mono">{{ run.target_path }}</span>
            <div
              v-if="run.error_message"
              class="cell-muted"
            >
              {{ run.error_message }}
            </div>
          </td>
          <td>
            <span
              class="badge"
              :class="[
                badgeClass(restoreStatusTone(run.status)),
                { 'badge--pulse': run.status === 'running' },
              ]"
            >
              <span
                v-if="run.status === 'running'"
                class="badge-dot"
              />
              {{ restoreStatusLabel(run.status) }}
            </span>
          </td>
          <td class="cell-muted">
            {{ run.requested_by }}
          </td>
          <td>
            <button
              v-if="run.status === 'pending'"
              class="btn btn-xs btn-ghost"
              :disabled="cancelling === run.id"
              @click="cancel(run)"
            >
              {{ cancelling === run.id ? 'Cancelling...' : 'Cancel' }}
            </button>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
