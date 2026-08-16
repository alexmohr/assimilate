<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { extractError } from '../utils/error'
import { useToast } from '../composables/useToast'
import { useEscapeKey } from '../composables/useEscapeKey'
import BaseModal from './BaseModal.vue'
import { repoOpLabel } from '../utils/repoOp'
import type { ActiveRepoOp, RepoWithStats } from '../types/repo'

const props = defineProps<{
  repo: RepoWithStats
  /** The operation borg is currently running, if any; blocks Break Lock. */
  currentOp: ActiveRepoOp | null
}>()

const emit = defineEmits<{ error: [message: string]; changed: [] }>()

const router = useRouter()
const { success: toastSuccess, error: toastError } = useToast()

const showConfirmRelocationDialog = ref(false)
const confirmRelocationLoading = ref(false)
const confirmRelocationError = ref<string | null>(null)
const confirmRelocationResult = ref<string | null>(null)

const showBreakLockDialog = ref(false)
const breakLockLoading = ref(false)
const breakLockError = ref<string | null>(null)
const breakLockResult = ref<string | null>(null)

const showRemoveDialog = ref(false)
const removeLoading = ref(false)

const showDeleteDialog = ref(false)
const deleteLoading = ref(false)

const showResetAndSyncDialog = ref(false)
const resetAndSyncLoading = ref(false)

useEscapeKey(showBreakLockDialog, () => {
  if (!breakLockLoading.value) showBreakLockDialog.value = false
})

async function doConfirmRelocation(): Promise<void> {
  confirmRelocationLoading.value = true
  confirmRelocationError.value = null
  confirmRelocationResult.value = null
  try {
    const res = await apiClient.post<{ message: string }>(
      `/repos/${props.repo.id}/confirm-relocation`,
    )
    confirmRelocationResult.value = res.data.message
    // The flag lives on the repo row the parent owns, so ask it to refresh
    // rather than mutating a prop.
    emit('changed')
  } catch (e: unknown) {
    confirmRelocationError.value = extractError(e)
  } finally {
    confirmRelocationLoading.value = false
  }
}

async function confirmBreakLock(): Promise<void> {
  breakLockLoading.value = true
  breakLockError.value = null
  breakLockResult.value = null
  try {
    const res = await apiClient.post<{ message: string; borg_output: string }>(
      `/repos/${props.repo.id}/break-lock`,
    )
    // borg_output carries the actual detail of what happened - notably
    // whether a stale local cache lock was found and cleared (or found but
    // left in place) - which message alone never conveys; it's the static
    // "lock broken on repository '<name>'" confirmation every time.
    breakLockResult.value = res.data.borg_output
      ? `${res.data.message}\n${res.data.borg_output}`
      : res.data.message
  } catch (e: unknown) {
    breakLockError.value = extractError(e)
  } finally {
    breakLockLoading.value = false
  }
}

async function confirmRemove(): Promise<void> {
  removeLoading.value = true
  try {
    await apiClient.delete(`/repos/${props.repo.id}`)
    showRemoveDialog.value = false
    void router.push('/repos')
  } catch (e: unknown) {
    emit('error', extractError(e))
  } finally {
    removeLoading.value = false
  }
}

async function confirmDelete(): Promise<void> {
  deleteLoading.value = true
  try {
    await apiClient.post(`/repos/${props.repo.id}/destroy`)
    showDeleteDialog.value = false
    void router.push('/repos')
  } catch (e: unknown) {
    emit('error', extractError(e))
  } finally {
    deleteLoading.value = false
  }
}

async function resetAndSync(): Promise<void> {
  showResetAndSyncDialog.value = false
  resetAndSyncLoading.value = true
  try {
    await apiClient.post(`/repos/${props.repo.id}/reset-and-sync?build_index=true`)
    toastSuccess('Archive metadata reset and re-import started. Progress is shown via WebSocket.')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    resetAndSyncLoading.value = false
  }
}
</script>

<template>
  <div class="info-card danger-zone">
    <h3 class="info-title">Danger Zone</h3>
    <div class="danger-body">
      <div class="danger-info">
        <span class="danger-heading">Confirm Repository Relocation</span>
        <span class="danger-desc">
          Allow the next backup to accept this repository at its current location. Use this when
          borg reports the repository was previously at a different path. The flag is cleared
          automatically after the backup succeeds.
        </span>
      </div>
      <div class="danger-action-wrap">
        <button
          class="btn btn-sm btn-danger"
          :disabled="confirmRelocationLoading"
          @click="showConfirmRelocationDialog = true"
        >
          {{ confirmRelocationLoading ? 'Confirming...' : 'Confirm Relocation' }}
        </button>
        <span
          v-if="repo.relocation_pending"
          class="danger-hint"
        >
          Relocation already pending — will apply on the next backup run.
        </span>
      </div>
    </div>
    <div class="danger-body">
      <div class="danger-info">
        <span class="danger-heading">Break Repository Lock</span>
        <span class="danger-desc">
          Remove a stale lock from the repository, including a stale local cache lock left behind by
          a crashed or forcibly killed backup process. Using this while a backup is in progress will
          corrupt the repository.
        </span>
      </div>
      <div class="danger-action-wrap">
        <button
          class="btn btn-sm btn-danger"
          :disabled="!!currentOp || breakLockLoading"
          :title="currentOp ? repoOpLabel(currentOp) : undefined"
          @click="showBreakLockDialog = true"
        >
          {{ breakLockLoading ? 'Breaking...' : 'Break Lock' }}
        </button>
        <span
          v-if="currentOp"
          class="danger-hint"
        >
          {{ repoOpLabel(currentOp) }}
        </span>
      </div>
    </div>
    <div class="danger-body">
      <div class="danger-info">
        <span class="danger-heading">Remove Repository</span>
        <span class="danger-desc"
          >Remove this repository from the UI and database. All associated schedules will be
          <strong>disabled</strong> and their repository link removed — they must be fixed manually.
          Reports will be deleted. The repository data on disk is NOT touched.</span
        >
      </div>
      <button
        class="btn btn-sm btn-danger"
        @click="showRemoveDialog = true"
      >
        Remove Repository
      </button>
    </div>
    <div class="danger-body">
      <div class="danger-info">
        <span class="danger-heading">Delete Repository</span>
        <span class="danger-desc"
          >PERMANENTLY DESTROY this repository from disk (rm -rf via SSH). This is irreversible and
          all backup data will be lost forever.</span
        >
      </div>
      <button
        class="btn btn-sm btn-danger"
        @click="showDeleteDialog = true"
      >
        Delete Repository
      </button>
    </div>
    <div class="danger-body">
      <div class="danger-info">
        <span class="danger-heading">Reset &amp; Re-import</span>
        <span class="danger-desc">
          Delete ALL archive metadata (backup reports, file indexes, tags) and re-import from the
          borg repository on disk. Use this when archives show as unmatched despite matching
          hostnames. The repository data on disk is NOT touched.
        </span>
      </div>
      <button
        class="btn btn-sm btn-danger"
        :disabled="resetAndSyncLoading"
        @click="showResetAndSyncDialog = true"
      >
        {{ resetAndSyncLoading ? 'Resetting...' : 'Reset &amp; Re-import' }}
      </button>
    </div>
  </div>

  <!-- Delete Confirmation Dialog -->
  <BaseModal
    :open="showDeleteDialog"
    title="⚠️ DESTROY Repository From Disk"
    @close="showDeleteDialog = false"
  >
    <p style="color: var(--danger); font-weight: 600">
      This will PERMANENTLY DELETE all data for
      <strong>{{ repo.name }}</strong> from the remote filesystem. This action is irreversible. All
      backup archives will be lost forever.
    </p>
    <p>
      The repository at <code>{{ repo.repo_path }}</code> on <code>{{ repo?.ssh_host }}</code> will
      be removed using <code>rm -rf</code>.
    </p>
    <p>
      All associated schedules will be <strong>disabled</strong> and their repository link removed.
      They will need to be reassigned or deleted manually.
    </p>

    <template #footer>
      <button
        class="btn btn-ghost"
        @click="showDeleteDialog = false"
      >
        Cancel
      </button>
      <button
        class="btn btn-danger"
        :disabled="deleteLoading"
        @click="confirmDelete"
      >
        {{ deleteLoading ? 'Destroying...' : 'Destroy Forever' }}
      </button>
    </template>
  </BaseModal>

  <!-- Remove (DB only) Confirmation Dialog -->
  <BaseModal
    :open="showRemoveDialog"
    title="Remove Repository"
    @close="showRemoveDialog = false"
  >
    <p>
      Are you sure you want to remove <strong>{{ repo.name }}</strong> from the database?
    </p>
    <p>
      All associated schedules will be <strong>disabled</strong> and their repository link removed.
      They will need to be reassigned or deleted manually. Reports will be deleted.
    </p>
    <p>The repository data on disk will NOT be touched.</p>

    <template #footer>
      <button
        class="btn btn-ghost"
        @click="showRemoveDialog = false"
      >
        Cancel
      </button>
      <button
        class="btn btn-danger"
        :disabled="removeLoading"
        @click="confirmRemove"
      >
        {{ removeLoading ? 'Removing...' : 'Remove' }}
      </button>
    </template>
  </BaseModal>

  <!-- Confirm Relocation Dialog -->
  <BaseModal
    :open="showConfirmRelocationDialog"
    title="Confirm Repository Relocation"
    @close="showConfirmRelocationDialog = false"
  >
    <p class="break-lock-warning">
      This sets <code>BORG_RELOCATED_REPO_ACCESS_IS_OK=yes</code> for the next backup run, allowing
      borg to accept the repository at its new location. Only confirm if you intentionally moved or
      re-pathed the repository.
    </p>
    <div
      v-if="confirmRelocationResult"
      class="break-lock-success"
    >
      {{ confirmRelocationResult }}
    </div>
    <div
      v-if="confirmRelocationError"
      class="form-error"
    >
      {{ confirmRelocationError }}
    </div>

    <template #footer>
      <button
        class="btn btn-ghost"
        @click="showConfirmRelocationDialog = false"
      >
        {{ confirmRelocationResult ? 'Close' : 'Cancel' }}
      </button>
      <button
        v-if="!confirmRelocationResult"
        class="btn btn-danger"
        :disabled="confirmRelocationLoading"
        @click="doConfirmRelocation"
      >
        {{ confirmRelocationLoading ? 'Confirming...' : 'Yes, Confirm Relocation' }}
      </button>
    </template>
  </BaseModal>

  <!-- Reset & Re-import Confirmation Dialog -->
  <BaseModal
    :open="showResetAndSyncDialog"
    title="Reset &amp; Re-import?"
    @close="showResetAndSyncDialog = false"
  >
    <p style="color: var(--danger); font-weight: 600">
      This will permanently delete ALL archive metadata for
      <strong>{{ repo.name }}</strong> and re-import from borg. This operation cannot be undone.
    </p>
    <p>
      Backup reports, file indexes, tags, and archive paths will be deleted. The repository data on
      disk (borg archives themselves) is NOT touched.
    </p>

    <template #footer>
      <button
        class="btn btn-ghost"
        @click="showResetAndSyncDialog = false"
      >
        Cancel
      </button>
      <button
        class="btn btn-danger"
        :disabled="resetAndSyncLoading"
        @click="resetAndSync"
      >
        {{ resetAndSyncLoading ? 'Resetting...' : 'Confirm Reset' }}
      </button>
    </template>
  </BaseModal>

  <!-- Break Lock Confirmation Dialog -->
  <BaseModal
    :open="showBreakLockDialog"
    title="Break Repository Lock"
    @close="showBreakLockDialog = false"
  >
    <p class="break-lock-warning">
      This will forcibly remove the lock from the repository, and clear any stale local cache lock
      found for it. Only use this if you are certain no backup is currently running. Breaking a lock
      during an active backup
      <strong>will corrupt the repository</strong>.
    </p>
    <div
      v-if="breakLockResult"
      class="break-lock-success"
    >
      {{ breakLockResult }}
    </div>
    <div
      v-if="breakLockError"
      class="form-error"
    >
      {{ breakLockError }}
    </div>

    <template #footer>
      <button
        class="btn btn-ghost"
        @click="showBreakLockDialog = false"
      >
        {{ breakLockResult ? 'Close' : 'Cancel' }}
      </button>
      <button
        v-if="!breakLockResult"
        class="btn btn-danger"
        :disabled="breakLockLoading"
        @click="confirmBreakLock"
      >
        {{ breakLockLoading ? 'Breaking Lock...' : 'Yes, Break Lock' }}
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.break-lock-success {
  margin-top: 0.75rem;
  padding: 0.75rem;
  background: var(--success-subtle);
  border-radius: var(--radius-sm);
  font-size: var(--fs-base);
  color: var(--success);
  white-space: pre-line;
}

.break-lock-warning {
  color: var(--danger);
  font-size: var(--fs-base);
  line-height: 1.5;
}

.danger-hint {
  font-size: var(--fs-2xs);
  color: var(--warning);
  text-align: right;
  max-width: 180px;
}
</style>
