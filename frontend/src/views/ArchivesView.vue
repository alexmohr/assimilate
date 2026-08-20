<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { storeToRefs } from 'pinia'
import { RefreshCw } from '@lucide/vue'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useClipboard } from '../composables/useClipboard'
import { extractError } from '../utils/error'
import ArchiveExplorer from '../components/ArchiveExplorer.vue'
import RestoreWizard from '../components/RestoreWizard.vue'
import ArchiveDiff from '../components/ArchiveDiff.vue'
import FileSearch from '../components/FileSearch.vue'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import type { Repo } from '../types/repo'
import BaseModal from '../components/BaseModal.vue'

const repos = ref<Repo[]>([])
const reposLoading = ref(false)
const reposError = ref<string | null>(null)
const selectedRepoId = ref<number | null>(null)

const archives = ref<ArchiveEntry[]>([])
const archivesLoading = ref(false)
const archivesError = ref<string | null>(null)
const selectedArchive = ref<ArchiveEntry | null>(null)

const showPassphraseDialog = ref(false)

const sortedArchives = computed(() =>
  [...archives.value].sort((a, b) => b.start.localeCompare(a.start)),
)

const { isAdmin } = storeToRefs(useAuthStore())

const selectedRepoName = computed(
  () => repos.value.find((r) => r.id === selectedRepoId.value)?.name ?? '',
)

async function loadRepos(): Promise<void> {
  reposLoading.value = true
  reposError.value = null
  try {
    const res = await apiClient.get<Repo[]>('/repos')
    repos.value = res.data
  } catch (e: unknown) {
    reposError.value = extractError(e)
  } finally {
    reposLoading.value = false
  }
}

async function onRepoChange(): Promise<void> {
  archives.value = []
  // Clearing the selection resets the browser: it watches `archive` and tears
  // down its own polling and path state.
  selectedArchive.value = null
  archivesError.value = null
  if (selectedRepoId.value === null) return
  await loadArchives()
}

// `silent` keeps the list on screen while a background refetch is in flight -
// the explorer asks for one after a delete, and blanking the panel would hide
// the row state it is refreshing.
async function loadArchives(silent = false): Promise<void> {
  if (selectedRepoId.value === null) return
  if (!silent) archivesLoading.value = true
  archivesError.value = null
  try {
    const res = await apiClient.get<ArchiveEntry[]>(`/repos/${selectedRepoId.value}/archives`)
    archives.value = res.data
  } catch (e: unknown) {
    archivesError.value = extractError(e)
  } finally {
    archivesLoading.value = false
  }
}

const showRestoreWizard = ref(false)
const showArchiveDiff = ref(false)

const passphrase = ref<string | null>(null)
const passphraseLoading = ref(false)
const passphraseError = ref<string | null>(null)
const { copied: passphraseCopied, copy: copyToClipboard } = useClipboard()

useEscapeKey(showPassphraseDialog, () => {
  showPassphraseDialog.value = false
})

async function revealPassphrase(): Promise<void> {
  if (selectedRepoId.value === null) return
  passphraseLoading.value = true
  passphraseError.value = null
  passphrase.value = null
  passphraseCopied.value = false
  try {
    const res = await apiClient.get<{ passphrase: string }>(
      `/repos/${selectedRepoId.value}/passphrase`,
    )
    passphrase.value = res.data.passphrase
    showPassphraseDialog.value = true
  } catch (e: unknown) {
    passphraseError.value = extractError(e)
    showPassphraseDialog.value = true
  } finally {
    passphraseLoading.value = false
  }
}

onMounted(loadRepos)
</script>

<template>
  <div class="archives-view">
    <div class="page-header">
      <h1 class="page-title">Archives</h1>
    </div>

    <div
      v-if="reposLoading"
      class="state-msg state-msg--inline"
    >
      Loading repositories...
    </div>
    <div
      v-else-if="reposError"
      class="error-banner"
    >
      {{ reposError }}
    </div>
    <template v-else>
      <div class="repo-selector">
        <label class="selector-label">Repository</label>
        <select
          v-model="selectedRepoId"
          class="input select-input select-input--lg"
          @change="onRepoChange"
        >
          <option
            :value="null"
            disabled
          >
            — select a repository —
          </option>
          <option
            v-for="repo in repos"
            :key="repo.id"
            :value="repo.id"
          >
            {{ repo.name }}
          </option>
        </select>
        <span
          v-if="repos.length === 0"
          class="muted-hint"
          >No repositories configured yet.</span
        >
        <button
          v-if="selectedRepoId !== null"
          class="btn btn-sm btn-ghost passphrase-btn"
          :disabled="passphraseLoading"
          @click="revealPassphrase"
        >
          {{ passphraseLoading ? 'Loading...' : 'Show Passphrase' }}
        </button>
      </div>

      <ArchiveExplorer
        v-if="selectedRepoId !== null"
        v-model:selected="selectedArchive"
        :repo-id="selectedRepoId"
        :repo-name="selectedRepoName"
        :archives="sortedArchives"
        :loading="archivesLoading"
        :error="archivesError"
        :is-admin="isAdmin"
        :reload="loadArchives"
        empty-description="This repository has no archives yet. They appear here once a backup has run."
      >
        <template #actions>
          <button
            class="btn btn-sm btn-ghost"
            :disabled="archives.length < 1"
            @click="showRestoreWizard = true"
          >
            Restore
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :disabled="archives.length < 2"
            @click="showArchiveDiff = true"
          >
            Diff
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :disabled="archivesLoading"
            aria-label="Refresh archives"
            @click="loadArchives()"
          >
            <RefreshCw
              :size="14"
              :class="{ spinning: archivesLoading }"
            />
          </button>
        </template>
      </ArchiveExplorer>

      <FileSearch
        :repo-id="selectedRepoId"
        :archives="archives.map((a) => ({ name: a.name }))"
      />
    </template>

    <!-- Passphrase Dialog -->
    <BaseModal
      :open="showPassphraseDialog"
      :title="passphrase ? 'Repository Passphrase' : 'Error'"
      @close="showPassphraseDialog = false"
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
          @click="showPassphraseDialog = false"
        >
          Done
        </button>
      </template>
    </BaseModal>
    <!-- Restore Wizard -->
    <RestoreWizard
      :open="showRestoreWizard"
      :repo-id="selectedRepoId"
      :archives="archives"
      @close="showRestoreWizard = false"
    />

    <!-- Archive Diff -->
    <ArchiveDiff
      :open="showArchiveDiff"
      :repo-id="selectedRepoId"
      :archives="archives"
      @close="showArchiveDiff = false"
    />
  </div>
</template>

<style scoped>
.archives-view {
  max-width: 1300px;
  color: var(--text-primary);
}

.repo-selector {
  display: flex;
  align-items: center;
  gap: var(--space-6);
  margin-bottom: var(--space-8);
}

.selector-label {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  white-space: nowrap;
}

.muted-hint {
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

/* Buttons */

.passphrase-btn {
  margin-left: auto;
}
</style>
