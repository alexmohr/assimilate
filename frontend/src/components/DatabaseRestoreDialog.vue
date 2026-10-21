<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { restoreArchiveDatabase } from '../api/archives'
import { useAsyncAction } from '../composables/useAsyncAction'
import { useRestoreRun } from '../composables/useRestoreRun'
import type { ArchiveDatabaseDump, DatabaseRestoreTarget } from '../types/generated'
import { badgeClass } from '../utils/badge'
import {
  newDatabaseLocation,
  newDatabaseNameProblem,
  replaceWarning,
  restoredDatabaseName,
} from '../utils/databaseRestore'
import { sourceName } from '../utils/databaseRuntimes'
import { restoreScope, restoreStatusLabel, restoreStatusTone } from '../utils/restoreRun'
import BaseModal from './BaseModal.vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'

/**
 * Restores one database dump from an archive, into a new database beside
 * the original or over the original itself, then follows the restore.
 */
const props = defineProps<{
  open: boolean
  repoId: number
  archiveName: string
  /** The dump to restore; null while the dialog is closed. */
  dump: ArchiveDatabaseDump | null
  /** The archive's own host, where the restore runs unless another is named. */
  defaultHost: string
  /** That host's domain, sent only while the restore stays on it. */
  defaultDomain: string | null
}>()

const emit = defineEmits<{ close: [] }>()

type TargetMode = DatabaseRestoreTarget['mode']

const hostname = ref('')
const mode = ref<TargetMode>('new_database')
const newName = ref('')
const confirmName = ref('')
const { loading: submitting, error, run: runAction } = useAsyncAction()
const { run: restoreRun, follow } = useRestoreRun()

const database = computed<string>(() => props.dump?.database ?? '')

const modeOptions = computed<SegmentedOption<TargetMode>[]>(() => [
  { value: 'new_database', label: 'Into a new database' },
  { value: 'original', label: `Replace ${database.value}` },
])

const nameProblem = computed<string | null>(() => newDatabaseNameProblem(newName.value))

const target = computed<DatabaseRestoreTarget>(() =>
  mode.value === 'new_database'
    ? { mode: 'new_database', name: newName.value }
    : { mode: 'original' },
)

const canSubmit = computed<boolean>(() => {
  if (props.dump === null || hostname.value.trim() === '') return false
  return mode.value === 'new_database'
    ? nameProblem.value === null
    : confirmName.value === database.value
})

function reset(): void {
  hostname.value = props.defaultHost
  mode.value = 'new_database'
  newName.value = props.dump === null ? '' : restoredDatabaseName(props.dump)
  confirmName.value = ''
  error.value = null
  restoreRun.value = null
}

watch(
  () => props.open,
  (open) => {
    if (open) reset()
  },
  { immediate: true },
)

async function submit(): Promise<void> {
  const dump = props.dump
  if (dump === null) return
  const host = hostname.value.trim()
  await runAction(async () => {
    follow(
      await restoreArchiveDatabase(props.repoId, props.archiveName, {
        archive_path: dump.archive_path,
        target: target.value,
        hostname: host,
        // The domain belongs to the archive's host; another host is named bare.
        domain: host === props.defaultHost ? props.defaultDomain : null,
      }),
    )
  })
}
</script>

<template>
  <BaseModal
    :open="open"
    title="Restore database"
    size="md"
    @close="emit('close')"
  >
    <div
      v-if="restoreRun"
      class="db-restore-status"
      data-testid="db-restore-status"
    >
      <span
        class="badge"
        :class="badgeClass(restoreStatusTone(restoreRun.status))"
        >{{ restoreStatusLabel(restoreRun.status) }}</span
      >
      <p v-if="restoreRun.status === 'pending'">
        {{ restoreRun.hostname }} is offline. The restore starts when it connects again.
      </p>
      <p v-else-if="restoreRun.status === 'running'">
        Restoring {{ restoreScope(restoreRun) }} with {{ restoreRun.hostname }}. You can close this
        window; the restore keeps running and is listed under Activity Log, Restores.
      </p>
      <p v-else-if="restoreRun.status === 'success'">
        Restored {{ restoreScope(restoreRun) }} with {{ restoreRun.hostname }}.
      </p>
      <p
        v-else-if="restoreRun.status === 'failed'"
        class="form-error"
      >
        {{ restoreRun.error_message ?? 'The restore failed.' }}
      </p>
      <p v-else>The restore was cancelled.</p>
    </div>

    <div
      v-else-if="dump"
      class="form-stack"
    >
      <dl class="info-grid">
        <dt>Database</dt>
        <dd class="mono">{{ dump.database }}</dd>
        <dt>Source</dt>
        <dd class="mono">{{ sourceName(dump) }}</dd>
      </dl>

      <div class="field">
        <label
          class="field-label"
          for="db-restore-host"
        >
          Restore with host
        </label>
        <input
          id="db-restore-host"
          v-model="hostname"
          class="input"
          type="text"
        />
        <span class="field-hint">
          The agent that runs the restore. It reaches the source the way its own database settings
          say, so that runtime has to be turned on there.
        </span>
      </div>

      <div class="field">
        <span class="field-label">Restore to</span>
        <BaseSegmented
          v-model="mode"
          :options="modeOptions"
          label="Where the dump is restored"
        />
      </div>

      <div
        v-if="mode === 'new_database'"
        class="field"
      >
        <label
          class="field-label"
          for="db-restore-name"
        >
          New database name
        </label>
        <input
          id="db-restore-name"
          v-model="newName"
          class="input mono"
          type="text"
          :aria-invalid="nameProblem !== null"
        />
        <span
          v-if="nameProblem"
          class="field-hint field-hint-error"
          data-testid="db-restore-name-problem"
          >{{ nameProblem }}</span
        >
        <span
          v-else
          class="field-hint"
        >
          {{ newDatabaseLocation(dump, newName) }}, which is left as it is.
        </span>
      </div>

      <div
        v-else
        class="field"
      >
        <p
          class="field-hint field-hint-error"
          data-testid="db-restore-replace-warning"
        >
          {{ replaceWarning(dump) }}
        </p>
        <label
          class="field-label"
          for="db-restore-confirm"
        >
          Type {{ dump.database }} to confirm
        </label>
        <input
          id="db-restore-confirm"
          v-model="confirmName"
          class="input mono"
          type="text"
          autocomplete="off"
        />
      </div>
    </div>

    <template #footer>
      <div
        v-if="error"
        class="form-error"
      >
        {{ error }}
      </div>
      <button
        v-if="restoreRun"
        type="button"
        class="btn btn-primary"
        @click="emit('close')"
      >
        Done
      </button>
      <template v-else>
        <button
          type="button"
          class="btn btn-ghost"
          @click="emit('close')"
        >
          Cancel
        </button>
        <button
          type="button"
          class="btn"
          :class="mode === 'new_database' ? 'btn-primary' : 'btn-danger'"
          :disabled="!canSubmit || submitting"
          data-testid="db-restore-submit"
          @click="submit"
        >
          {{
            submitting
              ? 'Restoring...'
              : mode === 'new_database'
                ? 'Restore'
                : `Replace ${database}`
          }}
        </button>
      </template>
    </template>
  </BaseModal>
</template>

<style scoped>
.db-restore-status {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-5);
}
</style>
