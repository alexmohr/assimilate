<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { RotateCcw } from '@lucide/vue'
import { cancelRestoreRun, getRestoreRun, listRestoreRuns } from '../api/restores'
import { useToast } from '../composables/useToast'
import { useWebSocket } from '../composables/useWebSocket'
import type { RestoreRun } from '../types/generated'
import { badgeClass } from '../utils/badge'
import { extractError } from '../utils/error'
import { formatDate, formatDateShort } from '../utils/format'
import { logger } from '../utils/logger'
import {
  isRestoreFinished,
  restoreScope,
  restoreStatusLabel,
  restoreStatusTone,
} from '../utils/restoreRun'
import BaseSpinner from './BaseSpinner.vue'
import EmptyState from './EmptyState.vue'
import LogList from './LogList.vue'
import LogRow from './LogRow.vue'

/**
 * Restores of archive files and database dumps onto agents, newest first, kept current from
 * the server's `RestoreRunChanged` pushes. A push only names the restore
 * and its status, so the restore is re-read from the admin-only API.
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

/**
 * Puts `run` in place of its older self, or at the top if it is new. Re-reads
 * can resolve out of order, so one that would reopen a restore already shown
 * as over is dropped: no later push would correct it.
 */
function upsert(run: RestoreRun): void {
  const index = runs.value.findIndex((r) => r.id === run.id)
  const shown = runs.value[index]
  if (shown !== undefined && isRestoreFinished(shown.status) && !isRestoreFinished(run.status)) {
    return
  }
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

/** Re-reads the restore a push names and shows it as it now stands. */
async function reload(id: string): Promise<void> {
  try {
    upsert(await getRestoreRun(id))
  } catch (e: unknown) {
    logger.warn('could not read the restore', e)
  }
}

/** Where the restore wrote to: a database's own name, or the target folder. */
function restoreInto(run: RestoreRun): string | null {
  if (run.database !== undefined) return null
  return run.target_path === null ? null : `into ${run.target_path}`
}

const expandedId = ref<string | null>(null)

function toggle(run: RestoreRun): void {
  expandedId.value = expandedId.value === run.id ? null : run.id
}

onMessage('RestoreRunChanged', (payload) => {
  reload(payload.id).catch(logger.error)
})

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
    description="Restores of archive files or database dumps onto an agent will appear here."
  />

  <LogList
    v-else
    data-testid="restore-runs"
  >
    <LogRow
      v-for="run in runs"
      :key="run.id"
      :data-restore-id="run.id"
      :tone="restoreStatusTone(run.status)"
      :title="run.hostname"
      :subtitle="run.archive_name"
      :facts="[restoreScope(run), run.error_message ?? restoreInto(run), `by ${run.requested_by}`]"
      :time="formatDateShort(run.created_at)"
      :time-title="formatDate(run.created_at)"
      :expanded="expandedId === run.id"
      @toggle="toggle(run)"
    >
      <template
        v-if="run.status !== 'success'"
        #badges
      >
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
      </template>
      <template
        v-if="run.status === 'pending'"
        #actions
      >
        <button
          class="btn btn-sm btn-ghost"
          data-testid="restore-cancel"
          :disabled="cancelling === run.id"
          @click="cancel(run)"
        >
          {{ cancelling === run.id ? 'Cancelling...' : 'Cancel' }}
        </button>
      </template>
      <template #detail>
        <dl class="info-grid">
          <dt>Status</dt>
          <dd>{{ restoreStatusLabel(run.status) }}</dd>
          <dt>Requested</dt>
          <dd>{{ formatDate(run.created_at) }} by {{ run.requested_by }}</dd>
          <template v-if="run.started_at">
            <dt>Started</dt>
            <dd>{{ formatDate(run.started_at) }}</dd>
          </template>
          <template v-if="run.finished_at">
            <dt>Finished</dt>
            <dd>{{ formatDate(run.finished_at) }}</dd>
          </template>
          <dt>Repository</dt>
          <dd>{{ run.repo_name }}</dd>
          <dt>Archive</dt>
          <dd class="restore-value">{{ run.archive_name }}</dd>
          <dt>{{ run.database ? 'Database' : 'Restoring' }}</dt>
          <dd class="restore-value">{{ restoreScope(run) }}</dd>
          <template v-if="run.database === undefined">
            <dt>Into</dt>
            <dd class="restore-value">{{ run.target_path ?? '\u2014' }}</dd>
          </template>
          <template v-if="run.files_restored !== null">
            <dt>Files restored</dt>
            <dd>{{ run.files_restored.toLocaleString() }}</dd>
          </template>
        </dl>
        <pre
          v-if="run.error_message"
          class="error-pre"
          >{{ run.error_message }}</pre
        >
      </template>
    </LogRow>
  </LogList>
</template>

<style scoped>
/* Paths and archive names are long unbroken tokens. */
.restore-value {
  font-family: var(--mono);
  word-break: break-all;
}
</style>
