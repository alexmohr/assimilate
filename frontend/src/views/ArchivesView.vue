<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { FilterMatchMode } from '@primevue/core/api'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import { RefreshCw, Check, AlertTriangle } from '@lucide/vue'
import { apiClient } from '../api/client'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useClipboard } from '../composables/useClipboard'
import { formatBytes, formatDate } from '../utils/format'
import { extractError } from '../utils/error'
import BaseSpinner from '../components/BaseSpinner.vue'
import ArchiveBrowserLayout from '../components/ArchiveBrowserLayout.vue'
import ArchiveFileBrowser from '../components/ArchiveFileBrowser.vue'
import RestoreWizard from '../components/RestoreWizard.vue'
import ArchiveDiff from '../components/ArchiveDiff.vue'
import FileSearch from '../components/FileSearch.vue'
import BaseHostLink from '../components/BaseHostLink.vue'
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

const archiveFilters = ref({
  name: { value: '', matchMode: FilterMatchMode.CONTAINS },
  start: { value: '', matchMode: FilterMatchMode.CONTAINS },
  hostname: { value: '', matchMode: FilterMatchMode.CONTAINS },
  original_size: { value: '', matchMode: FilterMatchMode.CONTAINS },
})

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

async function loadArchives(): Promise<void> {
  if (selectedRepoId.value === null) return
  archivesLoading.value = true
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
      class="state-msg state-error"
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

      <ArchiveBrowserLayout v-if="selectedRepoId !== null">
        <template #list>
          <div class="panel panel--sectioned archives-panel">
            <div class="panel-header">
              <span class="panel-title">Archives</span>
              <div class="panel-actions">
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
                  @click="loadArchives"
                >
                  <RefreshCw
                    :size="14"
                    :class="{ spinning: archivesLoading }"
                  />
                </button>
              </div>
            </div>

            <div
              v-if="archivesLoading"
              class="state-msg state-msg--inline"
            >
              <BaseSpinner size="sm" />
              Loading archives...
            </div>
            <div
              v-else-if="archivesError"
              class="state-msg state-error"
            >
              {{ archivesError }}
            </div>
            <div
              v-else-if="archives.length === 0"
              class="state-msg state-msg--inline"
            >
              No archives found.
            </div>
            <DataTable
              v-else
              v-model:filters="archiveFilters"
              :value="sortedArchives"
              :row-class="
                (data: ArchiveEntry) =>
                  selectedArchive?.name === data.name ? 'selected clickable' : 'clickable'
              "
              filter-display="row"
              table-class="data-table"
              @row-click="(e: { data: ArchiveEntry }) => (selectedArchive = e.data)"
            >
              <Column
                field="name"
                header="Name"
                :sortable="true"
                :show-filter-menu="false"
              >
                <template #filter="{ filterModel, filterCallback }">
                  <input
                    v-model="filterModel.value"
                    class="input filter-input"
                    type="text"
                    placeholder="Filter..."
                    @input="filterCallback()"
                  />
                </template>
                <template #body="{ data }">
                  <span class="td-mono">{{ data.name }}</span>
                </template>
              </Column>
              <Column
                field="start"
                header="Date"
                :sortable="true"
                :show-filter-menu="false"
              >
                <template #filter="{ filterModel, filterCallback }">
                  <input
                    v-model="filterModel.value"
                    class="input filter-input"
                    type="text"
                    placeholder="Filter..."
                    @input="filterCallback()"
                  />
                </template>
                <template #body="{ data }">
                  <span class="td-date">{{ formatDate(data.start) }}</span>
                </template>
              </Column>
              <Column
                field="hostname"
                header="Host"
                :sortable="true"
                :show-filter-menu="false"
              >
                <template #filter="{ filterModel, filterCallback }">
                  <input
                    v-model="filterModel.value"
                    class="input filter-input"
                    type="text"
                    placeholder="Filter..."
                    @input="filterCallback()"
                  />
                </template>
                <template #body="{ data }">
                  <BaseHostLink
                    v-if="data.matched === true && data.agent_hostname"
                    :hostname="data.agent_hostname"
                    class="host-link"
                    @click.stop
                  />
                  <BaseHostLink
                    v-else-if="data.matched !== true"
                    :hostname="data.hostname"
                    class="unmatched-host-link"
                    @click.stop
                  />
                  <span
                    v-else
                    class="td-host"
                    >{{ data.hostname }}</span
                  >
                </template>
              </Column>
              <Column
                field="matched"
                header=""
                style="width: 3rem"
              >
                <template #body="{ data }">
                  <span
                    v-if="data.matched === true"
                    class="match-icon match-ok"
                    title="Matched"
                  >
                    <Check :size="14" />
                  </span>
                  <span
                    v-else-if="data.matched !== true"
                    class="match-icon match-warn"
                    title="Unmatched"
                  >
                    <AlertTriangle :size="14" />
                  </span>
                </template>
              </Column>
              <Column
                field="original_size"
                header="Size"
                :sortable="true"
                :show-filter-menu="false"
              >
                <template #filter="{ filterModel, filterCallback }">
                  <input
                    v-model="filterModel.value"
                    class="input filter-input"
                    type="text"
                    placeholder="Filter..."
                    @input="filterCallback()"
                  />
                </template>
                <template #body="{ data }">
                  <span class="td-size">{{ formatBytes(data.original_size) }}</span>
                </template>
              </Column>
            </DataTable>
          </div>
        </template>

        <template #browser>
          <div class="panel panel--sectioned browser-panel">
            <ArchiveFileBrowser
              :repo-id="selectedRepoId"
              :archive="selectedArchive"
            />
          </div>
        </template>
      </ArchiveBrowserLayout>

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
  gap: 1rem;
  margin-bottom: 1.5rem;
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

.td-mono {
  font-family: var(--mono);
  font-size: var(--fs-sm);
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.td-host {
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

.unmatched-host-link {
  font-size: var(--fs-sm);
  color: var(--warning);
  text-decoration: none;
}

.unmatched-host-link:hover {
  text-decoration: underline;
}

.match-icon {
  font-size: var(--fs-md);
}

/* Buttons */

.passphrase-btn {
  margin-left: auto;
}
</style>
