<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { AlertTriangle } from '@lucide/vue'
import {
  resumeMaintenance,
  runMaintenance,
  updateMaintenance,
  keepArchive,
} from '../api/appendOnly'
import { extractError } from '../utils/error'
import { formatDate, relativeTime } from '../utils/format'
import { cronToHuman } from '../utils/cron'
import { useToast } from '../composables/useToast'
import ConfirmDeleteDialog from './ConfirmDeleteDialog.vue'
import CronBuilder from './CronBuilder.vue'
import PaneRow from './PaneRow.vue'
import RepoMaintenanceRuns from './RepoMaintenanceRuns.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { AppendOnlyStatusResponse, MaintenanceSettingsResponse } from '../types/generated'

/**
 * Maintenance for an append-only repository: the server connects with its
 * full-access key, compares borg's archive list with its own, checks the
 * repository, applies each schedule's retention and compacts. If an archive
 * vanished that Assimilate did not delete, it stops before writing anything.
 */
const props = defineProps<{
  repoId: number
  status: AppendOnlyStatusResponse
  maintenance: MaintenanceSettingsResponse
  /** Whether a maintenance run is in progress right now. */
  running: boolean
}>()

const emit = defineEmits<{ status: [value: AppendOnlyStatusResponse]; changed: [] }>()

const { success: toastSuccess, error: toastError } = useToast()

const cron = ref(props.maintenance.cron_expression)
const checkBeforePrune = ref(props.maintenance.check_before_prune)
const saving = ref(false)
const saveError = ref<string | null>(null)
const starting = ref(false)
const keeping = ref(false)

const resumeOpen = ref(false)
const resuming = ref(false)
const resumeError = ref<string | null>(null)

// A refresh from the server replaces what the form shows, unless the user is
// part-way through an edit - that is what `dirty` is for.
const dirty = computed<boolean>(
  () =>
    cron.value !== props.maintenance.cron_expression ||
    checkBeforePrune.value !== props.maintenance.check_before_prune,
)

watch(
  () => props.maintenance,
  (next, prev) => {
    if (cron.value === prev.cron_expression && checkBeforePrune.value === prev.check_before_prune) {
      cron.value = next.cron_expression
      checkBeforePrune.value = next.check_before_prune
    }
  },
)

async function save(): Promise<void> {
  saving.value = true
  saveError.value = null
  try {
    emit(
      'status',
      await updateMaintenance(props.repoId, {
        cron_expression: cron.value,
        check_before_prune: checkBeforePrune.value,
      }),
    )
    toastSuccess('Maintenance schedule saved.')
  } catch (e: unknown) {
    saveError.value = extractError(e)
  } finally {
    saving.value = false
  }
}

function discard(): void {
  cron.value = props.maintenance.cron_expression
  checkBeforePrune.value = props.maintenance.check_before_prune
  saveError.value = null
}

async function runNow(): Promise<void> {
  starting.value = true
  try {
    await runMaintenance(props.repoId)
    toastSuccess('Maintenance started.')
    emit('changed')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    starting.value = false
  }
}

async function resume(): Promise<void> {
  resuming.value = true
  resumeError.value = null
  try {
    emit('status', await resumeMaintenance(props.repoId))
    resumeOpen.value = false
    toastSuccess('Maintenance resumed.')
    emit('changed')
  } catch (e: unknown) {
    resumeError.value = extractError(e)
  } finally {
    resuming.value = false
  }
}

async function keep(archiveName: string): Promise<void> {
  keeping.value = true
  try {
    await keepArchive(props.repoId, archiveName)
    toastSuccess(`${archiveName} will be kept.`)
    emit('changed')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    keeping.value = false
  }
}
</script>

<template>
  <div class="panel-stack">
    <div class="pane-head">
      <p class="pane-lede">
        The server connects with the maintenance key, compares borg's archive list with
        Assimilate's, checks the repository, applies every schedule's retention policy and compacts.
        If an archive vanished that Assimilate did not delete, it stops before writing anything.
      </p>
      <button
        type="button"
        class="btn btn-primary"
        :disabled="starting || running || maintenance.stopped !== null"
        :title="maintenance.stopped ? 'Accept and resume first' : undefined"
        @click="runNow"
      >
        {{ starting || running ? 'Running...' : 'Run maintenance now' }}
      </button>
    </div>

    <div
      v-if="maintenance.stopped"
      class="callout callout--warning"
      role="alert"
    >
      <AlertTriangle :size="16" />
      <div class="stopped-body">
        <p>
          <strong>Maintenance stopped {{ relativeTime(maintenance.stopped.since) }}.</strong>
          These archives are gone from borg, but Assimilate never deleted them:
        </p>
        <ul class="stopped-list">
          <li
            v-for="name in maintenance.stopped.vanished_archives"
            :key="name"
            class="mono"
          >
            {{ name }}
          </li>
        </ul>
        <p class="muted">
          Nothing was written, so they can still be rolled back from the repository's transaction
          log on the host.
        </p>
        <button
          type="button"
          class="btn btn-sm"
          @click="resumeOpen = true"
        >
          Accept and resume
        </button>
      </div>
    </div>

    <div class="pane-rows">
      <PaneRow
        title="Run"
        stack
        help="maintenance schedule"
      >
        <template #help>
          When the server prunes, compacts and applies deletions you queued. Server time.
        </template>
        <CronBuilder v-model="cron" />
        <p class="field-hint">{{ cronToHuman(cron) }}</p>
      </PaneRow>
      <PaneRow
        title="Check before pruning"
        hint="borg check --repository-only"
      >
        <ToggleSwitch
          v-model="checkBeforePrune"
          label="Check before pruning"
        />
      </PaneRow>
      <PaneRow title="Next run">
        <span v-if="maintenance.next_run_at">{{ formatDate(maintenance.next_run_at) }}</span>
        <span
          v-else
          class="muted"
        >
          Not scheduled
        </span>
      </PaneRow>
      <PaneRow title="Last run">
        <span v-if="maintenance.last_run_at">{{ formatDate(maintenance.last_run_at) }}</span>
        <span
          v-else
          class="muted"
        >
          Never
        </span>
      </PaneRow>
    </div>
    <div
      v-if="saveError"
      class="form-error"
    >
      {{ saveError }}
    </div>
    <div
      v-if="dirty"
      class="panel-actions"
    >
      <button
        type="button"
        class="btn btn-ghost"
        :disabled="saving"
        @click="discard"
      >
        Cancel
      </button>
      <button
        type="button"
        class="btn btn-primary"
        :disabled="saving"
        @click="save"
      >
        {{ saving ? 'Saving...' : 'Save schedule' }}
      </button>
    </div>

    <RepoMaintenanceRuns
      :pending="status.pending_deletions"
      :runs="status.recent_runs"
      :busy="keeping"
      @keep="keep"
    />

    <ConfirmDeleteDialog
      :show="resumeOpen"
      title="Accept and resume maintenance"
      confirm-label="Accept and resume"
      submitting-label="Resuming..."
      :submitting="resuming"
      :error="resumeError"
      @cancel="resumeOpen = false"
      @confirm="resume"
    >
      Resuming accepts that the missing archives are gone. Their data is only really gone once
      maintenance next writes to the repository, so if the removal was not intended, roll the
      repository back on the host first.
    </ConfirmDeleteDialog>
  </div>
</template>

<style scoped>
.stopped-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  align-items: flex-start;
}

.stopped-body p {
  margin: 0;
}

.stopped-list {
  margin: 0;
  padding-left: var(--space-7);
}
</style>
