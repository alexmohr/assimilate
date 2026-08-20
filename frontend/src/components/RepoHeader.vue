<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { apiClient } from '../api/client'
import { formatBytes, relativeTime } from '../utils/format'
import { extractError } from '../utils/error'
import { useToast } from '../composables/useToast'
import { useClipboard } from '../composables/useClipboard'
import BaseModal from './BaseModal.vue'
import DetailHeader from './DetailHeader.vue'
import OverflowMenu from './OverflowMenu.vue'
import type { RepoWithStats } from '../types/repo'

/**
 * The repository detail page's identity block.
 *
 * Repositories were the one detail page with no header: the name appeared only
 * as the last breadcrumb, and every action lived in a bottom-right corner of
 * one of five stacked cards, so the page opened with no answer to "what is
 * this and what would I do to it".
 *
 * Sync and the import reset are here rather than on the settings pane because
 * they act on the repository as a whole; editing its connection details stays
 * with the fields it edits.
 */
const props = defineProps<{
  repo: RepoWithStats
  isAdmin: boolean
  /** Label for the running import phase; the parent tracks it over the socket. */
  importPhaseVerb: string
}>()

const emit = defineEmits<{ 'import-reset': [] }>()

const { success: toastSuccess, error: toastError } = useToast()
const { copied: passphraseCopied, copy: copyToClipboard } = useClipboard()

const syncLoading = ref(false)
const resetImportLoading = ref(false)

const showPassphraseDialog = ref(false)
const passphrase = ref<string | null>(null)
const passphraseLoading = ref(false)
const passphraseError = ref<string | null>(null)

const target = computed(
  () => `${props.repo.ssh_user}@${props.repo.ssh_host}:${props.repo.repo_path}`,
)

const importPercent = computed(() =>
  props.repo.import_total > 0
    ? Math.round((props.repo.import_progress / props.repo.import_total) * 100)
    : null,
)

const statusTone = computed(() => {
  if (props.repo.import_error) return 'badge--danger'
  if (props.repo.importing) return 'badge--warning badge--pulse'
  return props.repo.enabled ? 'badge--success' : 'badge--neutral'
})

const statusLabel = computed(() => {
  if (props.repo.import_error) return 'Import failed'
  if (props.repo.importing) {
    return props.repo.import_total > 0
      ? `${props.importPhaseVerb} ${props.repo.import_progress}/${props.repo.import_total}`
      : `${props.importPhaseVerb}...`
  }
  return props.repo.enabled ? 'Enabled' : 'Disabled'
})

async function syncRepo(): Promise<void> {
  syncLoading.value = true
  try {
    await apiClient.post(`/repos/${props.repo.id}/sync?build_index=true`)
    toastSuccess('Sync started. Archive contents are being indexed in the background.')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    syncLoading.value = false
  }
}

async function resetImport(): Promise<void> {
  resetImportLoading.value = true
  try {
    await apiClient.post(`/repos/${props.repo.id}/reset-import`)
    toastSuccess('Import state reset.')
    emit('import-reset')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    resetImportLoading.value = false
  }
}

/**
 * Drops the plaintext passphrase the moment its dialog is dismissed, rather
 * than leaving it in memory until the next reveal. The dialog's title is
 * driven by `passphraseError` rather than by the passphrase itself so that
 * wiping it here does not relabel the box while it fades out.
 */
function closePassphraseDialog(): void {
  showPassphraseDialog.value = false
  passphrase.value = null
  passphraseError.value = null
  passphraseCopied.value = false
}

async function revealPassphrase(): Promise<void> {
  passphraseLoading.value = true
  passphraseError.value = null
  passphrase.value = null
  passphraseCopied.value = false
  try {
    const res = await apiClient.get<{ passphrase: string }>(`/repos/${props.repo.id}/passphrase`)
    passphrase.value = res.data.passphrase
    showPassphraseDialog.value = true
  } catch (e: unknown) {
    passphraseError.value = extractError(e)
    showPassphraseDialog.value = true
  } finally {
    passphraseLoading.value = false
  }
}
</script>

<template>
  <DetailHeader
    :name="repo.name"
    mono
    :subtitle="target"
  >
    <template #badges>
      <span
        class="badge repo-status-badge"
        :class="statusTone"
        :title="repo.import_error ?? undefined"
      >
        {{ statusLabel }}
      </span>
    </template>

    <template #meta>
      <span>
        archives <b>{{ repo.archive_count }}</b>
      </span>
      <span>
        dedup <b>{{ formatBytes(repo.total_deduplicated_size) }}</b>
      </span>
      <span v-if="repo.last_backup_at">
        last write <b>{{ relativeTime(repo.last_backup_at) }}</b>
      </span>
      <span>
        compression <b>{{ repo.compression }}</b>
      </span>
      <span>
        encryption <b>{{ repo.encryption }}</b>
      </span>
    </template>

    <template
      v-if="isAdmin"
      #actions
    >
      <button
        v-if="repo.importing || repo.import_error"
        class="btn btn-sm btn-primary"
        :disabled="resetImportLoading"
        @click="resetImport"
      >
        {{ resetImportLoading ? 'Resetting...' : 'Cancel import' }}
      </button>
      <button
        v-else
        class="btn btn-sm btn-primary"
        :disabled="syncLoading"
        @click="syncRepo"
      >
        {{ syncLoading ? 'Syncing...' : 'Sync now' }}
      </button>

      <OverflowMenu
        v-slot="{ run }"
        label="More repository actions"
      >
        <button
          class="overflow-menu-item"
          role="menuitem"
          type="button"
          :disabled="passphraseLoading"
          @click="run(revealPassphrase)"
        >
          {{ passphraseLoading ? 'Loading...' : 'Show passphrase' }}
        </button>
      </OverflowMenu>
    </template>

    <!--
      An import is the one repository state worth watching rather than
      visiting, so its progress rides along under the header on every tab.
    -->
    <template
      v-if="repo.importing"
      #footer
    >
      <div class="detail-header-error repo-import">
        <div
          v-if="importPercent !== null"
          class="progress-row"
        >
          <div class="progress-track">
            <div
              class="progress-bar"
              :style="{ width: `${importPercent}%` }"
            ></div>
          </div>
          <span class="progress-label">{{ importPercent }}%</span>
        </div>
        <p
          v-if="repo.import_status_message"
          class="field-hint import-status-msg"
        >
          {{ repo.import_status_message }}
        </p>
      </div>
    </template>
  </DetailHeader>

  <BaseModal
    :open="showPassphraseDialog"
    :title="passphraseError ? 'Error' : 'Repository passphrase'"
    @close="closePassphraseDialog"
  >
    <template v-if="passphrase">
      <p class="passphrase-warning">Keep this passphrase secure. Do not share it.</p>
      <div class="passphrase-box">
        <code class="passphrase-text">{{ passphrase }}</code>
        <button
          class="btn btn-sm btn-ghost"
          @click="passphrase && copyToClipboard(passphrase)"
        >
          {{ passphraseCopied ? 'Copied!' : 'Copy' }}
        </button>
      </div>
    </template>
    <div
      v-else-if="passphraseError"
      class="form-error"
    >
      {{ passphraseError }}
    </div>

    <template #footer>
      <button
        class="btn btn-primary"
        @click="closePassphraseDialog"
      >
        Done
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.repo-import {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
</style>
